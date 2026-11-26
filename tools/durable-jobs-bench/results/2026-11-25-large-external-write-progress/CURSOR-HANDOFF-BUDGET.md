# How early the next LARGE cursor grant would need to arrive

For each of the eight measured ordinary span arms, the historical 20% throughput target for **drain** binds before the target for full submit, for both Oban and River. Even instantaneous producer work would leave that drain requirement intact. The comparison uses the peer deadlines and measured duplicate spreads from pilot `25282314`. The peer runs came from the earlier matched FILE series.

The two peers pose different problems. Seven of eight span arms already cleared River's economic thresholds on both clocks. EXTERNAL block 1 span A needed another **9.003 ms** of drain improvement. The pilot's product and bridge gates still control whether a fresh comparison can run. For Oban, all eight require substantial further gains. In three arms, even giving away the entire observed interval from each grant's earliest handler entry to the next grant's first stamp leaves a deficit.

| Storage / block | Oban drain improvement required, A / B (ms) | Remaining Oban deficit after full credit, A / B (ms) | Share of each interval required, A / B |
|---|---:|---:|---:|
| DEFAULT 1 | 213.964 / 172.474 | −28.712 / −20.575 | 87.36% / 89.34% |
| DEFAULT 2 | 190.172 / 241.402 | +13.392 / −13.121 | 107.67% / 94.84% |
| EXTERNAL 1 | 306.680 / 284.509 | +73.108 / +121.097 | 132.02% / 174.11% |
| EXTERNAL 2 | 145.870 / 127.127 | −66.795 / −71.671 | 68.59% / 63.95% |

Negative remaining deficits mean the modeled finish is early. Percentages above 100 exceed the time available under this model. Even in the best EXTERNAL block, closing the Oban gap on both duplicates would take at least 68.59% of each positive interval if all intervals could be shortened by the same fraction.

## Catching Oban at its observed speed

Setting aside both the 20% improvement target and the duplicate-spread allowance gives a second question. At Oban’s actual historical median drain of 506.301 ms on DEFAULT and 508.086 ms on EXTERNAL, how much work had Mesh started? Times are relative to each arm’s durable enqueue commit. Each pair of entries is span A / span B.

| Storage / block | Jobs still awaiting handler entry | Jobs still awaiting their durable effect | First handler in final grant, ms past Oban median | Time from that first entry to final settlement, ms |
|---|---:|---:|---:|---:|
| DEFAULT 1 | 14 / 8 | 18 / 8 | 76.857 / 27.106 | 11.233 / 19.494 |
| DEFAULT 2 | 9 / 17 | 11 / 17 | 49.591 / 87.757 | 4.967 / 18.031 |
| EXTERNAL 1 | 12 / 7 | 16 / 11 | 49.630 / 24.786 | 13.661 / 16.334 |
| EXTERNAL 2 | 8 / 6 | 8 / 6 | 20.960 / 10.240 | 21.486 / 13.463 |

Every final grant’s first handler entered after the corresponding Oban median. The last handler across all 64 jobs entered **14.187–87.757 ms** after that median. Even instantaneous completion from each recorded handler entry would miss equal drain throughput in all eight arms if earlier grant and entry times stayed fixed. In EXTERNAL block 2 span A, 62 jobs definitely had a claim stamp by the median and the remaining two fell in its millisecond of uncertainty. Eight handlers had yet to enter. This makes the interval from stamping through validated delivery and handler entry relevant alongside time spent obtaining the next queue turn. Earlier application completions could also free capacity and change subsequent entry times.

## Calculation and limits

The reducer verifies the official score and the raw SHA-256 for each measured span lane. It reuses the [unique grant partition](ORDINARY-GRANT-TURNS.md) for that exact raw digest. For each turn it takes the maximum settlement time across its own jobs. Each positive interval between the previous turn's earliest handler entry and the current turn's first stamp is eligible for credit. Crediting an interval moves all grants from that point forward. Earlier grants keep their own settlement times and can become the limiting turn. The solver takes the largest required fraction across **every** turn. Taking the last grant's deficit divided by all eligible time would miss that constraint.

The first stamp has millisecond resolution. At a fixed history, using its floor can shorten an eligible interval by less than one millisecond. The model preserves the observed enqueue time, owner, row count, service duration and settlement lag for each grant, and the released stock/span noise allowance. It assigns zero cost to obtaining a shorter interval. Independent application writes, SQL execution, and scheduling overlapped many of these intervals. Their service must still be paid on two shared cores. These numbers price a potential change in ordering. They do not establish time spent waiting for a lock or savings achievable by a particular implementation.

An admission change that preserves eight grants needs its own measured transfer across owners. A change that reduces the number of grants has different costs, including longer first grants, fewer active workers, and larger query results. Those two mechanisms can be tested separately against the original span duplicates. Neither the eight-arm model nor the historical peer thresholds qualify a product.

Reproduce the [per-turn calculation](cursor_handoff_budget.py) and [receipt](cursor-handoff-budget.json) from the repository root.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-large-external-write-progress
python3 -B "$d/cursor_handoff_budget.py" | cmp - "$d/cursor-handoff-budget.json"
```
