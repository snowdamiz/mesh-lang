# Where LARGE FILE enqueue slows down within a submission

In all 36 arms of producer-budget SETUP `a2a90174`, the five intervals from stored job 1 to job 6 took longer than the five from job 6 to job 11. The smallest difference was 20 ms. Every arm also spent longer between jobs 1 and 16 than between jobs 49 and 64. Each comparison uses the same number of intervals within one transaction on the same broker and PostgreSQL backend.

| Broker and recorder | Runs | Jobs 1–6 minus 6–11, ms (min / median / max) | Jobs 1–16 minus 49–64, ms (min / median / max) |
|---|---:|---:|---:|
| Original, off | 6 | 20 / 25.5 / 38 | 49 / 67 / 94 |
| Original, on | 6 | 21 / 23.5 / 32 | 44 / 76.5 / 104 |
| Relink A, off | 6 | 25 / 26.5 / 29 | 47 / 76.5 / 164 |
| Relink B, off | 6 | 22 / 28 / 34 | 39 / 73 / 74 |
| Relink A, on | 6 | 23 / 27.5 / 30 | 40 / 66 / 80 |
| Relink B, on | 6 | 24 / 29 / 33 | 28 / 79.5 / 120 |

The original broker with recording off had typical intervals of **15, 11, 9, 8, 7 and 6.5 ms** for intervals ending at ranks 2–6, 7–11, 12–16, 17–32, 33–48 and 49–64. Each number is a median of the six within-run medians. Matching the final 15 intervals on the first 15 would save a median 67 ms on that broker, with a low of 49 ms. The 60 ms enqueue gate requires a 4 ms mean improvement per early interval if all the improvement comes from those first 15. Clearing the independent full-submit gate also requires the gains to survive through settlement.

## The largest identical-broker difference

In group 3, OFF relink A took 430.568 ms longer to enqueue than OFF relink B. Of that difference, 34.221 ms accrued before the first recorded stamp, **395 ms (91.7%)** between stamps 1 and 64, and 1.347 ms from the last stamp to enqueue commit. The recurring difference accumulated over five consecutive intervals of ranks.

| Stamp ranks | A minus B, ms |
|---|---:|
| 1–8 | 51 |
| 8–16 | 107 |
| 16–32 | 64 |
| 32–48 | 78 |
| 48–64 | 95 |

A was slower by at least two recorded milliseconds on **58 of 63** individual intervals. B was slower by that amount on two. This pair shows that arm-wide variation can affect the recurring SQL and broker path across almost the whole batch. Removing one slow first call cannot account for its spread.

## What the timestamps resolve

The SQL insert records whole milliseconds. Subtracting two equal-width interval durations introduces less than 2 ms of rounding uncertainty. Adjacent stamps span the previous row's remaining executor and protocol work, the broker's preparation of the next job, its wire exchanges, and the next scalar call up to its stamp. These data identify the ranks at which to measure those costs. They give no causal assignment to JSON conversion, cached plans, allocator behavior, WAL or scheduling. A diagnostic can distinguish work that contracts in the first 15 intervals from work of similar cost at every rank by aligning its boundaries to the durable row IDs.

The canonical controller release exited 1. The owner reproduced all 36 arms after checking exact decimal OID equality for the two representation differences. This reduction binds that archive and adjudication by Git commit, joins every stored row to the durable ID map and final ledger, and checks timestamp collection after settlement. The native and export statuses were both zero.

From the repository root, reproduce the [per-arm arithmetic](producer-rank-costs.json) with the [offline reducer](producer_rank_costs.py).

```sh
d=tools/durable-jobs-bench/results/2026-11-25-large-external-write-progress
python3 -B "$d/producer_rank_costs.py" | cmp - "$d/producer-rank-costs.json"
```
