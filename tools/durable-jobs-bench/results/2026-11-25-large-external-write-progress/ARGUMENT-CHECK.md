# LARGE argument checks during claim and settlement

On the public FILE path with symmetric EXTERNAL storage, reaching Oban's drain time would require advancing the last durable application write by **121–344 ms** in the five matched rotations, even if final settlement were instantaneous. At Oban's finish, Mesh still had 13–32 effects outstanding across two to four runner processes. Trimming just the interval after the last effect can save 1.7–10.6 ms. [Archive calculation](constraint_budget.py) · [All ten archive identities and deadlines](constraint-budget.json)

| Rotation | FILE drain, ms | Oban drain, ms | FILE time after last effect, ms | Effects outstanding at Oban finish | Advance needed at last effect with zero terminal tail, ms |
|---|---:|---:|---:|---:|---:|
| 1 | 689.3 | 491.7 | 3.3 | 24 | 194.3 |
| 2 | 648.8 | 485.6 | 3.9 | 24 | 159.3 |
| 3 | 635.1 | 511.8 | 2.1 | 18 | 121.3 |
| 4 | 862.5 | 508.1 | 10.6 | 32 | 343.9 |
| 5 | 659.1 | 536.2 | 1.7 | 13 | 121.2 |

Both clocks start at the arm's own enqueue commit. The arithmetic holds the observed effect schedule fixed when eliminating the final tail. Faster **earlier** settlements could still change contention or allow another handler to start sooner. The receipt also gives per-order deadlines for a 20% throughput lead, computed as five sixths of the peer elapsed time.

The original Jobs table checks the root JSON type of `args` on a candidate tuple. PostgreSQL 17.11's update path evaluates tuple constraints before replacing the heap tuple. Its built-in `jsonb_typeof` asks for a detoasted JSONB value. The first-attempt ledgers have 64 transitions into execution and 64 into completion. They identify 128 occasions to investigate whether checking an otherwise unchanged out-of-line argument fetches its contents. The archives do not observe the datum representation at those calls, cache reads per predicate, or elapsed time attributable to it. The native proof must distinguish work at claim, work at settlement, and any other consumer that already fetched the value. [Jobs schema](../../../../compiler/mesh-rt/src/jobs/schema.sql) · [PostgreSQL update path](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/executor/nodeModifyTable.c#L1980-L1987) · [PostgreSQL JSONB type check](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/utils/adt/jsonb.c#L217-L222)

The earlier 64-job SQL component already exercised the header-only check. It removed 4,192 TOAST heap hits in claim and another 4,192 in settlement. Claim elapsed time beat both duplicates in one of four rotations. Settlement saved 1.2–6.4 backend CPU-ms. These are historical component measurements on ARM64. The repeated page access is real, but neither its CPU saving nor its effect on the earlier native grants supports a separate native reservation. [Original physical witness](../2026-11-23-large-storage-component/README.md) · [Claim and settlement reanalysis](../2026-11-25-large-external-argcheck/README.md)

Run `python3 -B constraint_budget.py` from this directory to read the archived stock FILE and Oban ledgers. The calculation checks the per-arm release records, raw hashes, input payload identities, and 128 independent application commits per arm.

## Prepared span at 16 MiB

The PG17.11 proof at `11d96f4eb` recorded 693 shared buffer hits for eight activation UPDATEs including their triggers. Its separate zero in `pg_statio_user_tables` cannot establish how many of those accesses went to TOAST. The worker requested its next statistics flush while its transaction was open. The admin backend sampled before the worker rolled back. The flush request sets a flag, and the backend reports pending cumulative statistics on returning to idle outside a transaction. A comparison of completed transactions would need to account for PREPARE's own fetches as well. [Proof source at the original commit](../../../../../t3-45b6ea43/tools/durable-jobs-bench/results/2026-11-25-large-activation-sql/prove.py#L130) · [PG17.11 flush request](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/utils/activity/pgstat.c#L660-L668) · [PG17.11 backend reporting](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/tcop/postgres.c#L4330-L4381)

The archived mirrored span pilot gives an independent price for changing the UPDATE after each attempt stamp. Each of eight owners received eight rows in a separate serialized turn. Hold the turn order, enqueue, and each job's progress after its own receipt fixed. If every UPDATE could advance its turn and every subsequent turn by the same amount, a job owned by turn k could gain 8k times that amount. The table takes the largest required saving over **all 64 jobs**, using the pilot's observed margins. Values are microseconds per UPDATE needed to reach equality. Zero means the lane already reached that cutoff. The historical Oban columns charge the observed span duplicate spread.

| Block and lane | Product submit | Product drain | Product last effect | Loosest historical Oban drain | Tightest historical Oban drain |
|---|---:|---:|---:|---:|---:|
| Forward span A | 2,121 | 955 | 924 | 155 | 845 |
| Forward span B | 2,362 | 209 | 175 | 0 | 113 |
| Reverse span A | 0 | 0 | 0 | 274 | 1,027 |
| Reverse span B | 0 | 0 | 0 | 0 | 559 |

Forward span B still missed the historical product submit gate by **22.445 ms** even when each owner received credit for the *entire* interval from its first stamp floor through its checked receipt, plus the same credit for every preceding turn. The limiting job belonged to turn four. Those four turns supplied only **53.132 ms** of gross credit. The model even credits overlapping receipt time twice and gives activation the time spent in scheduling, transport and COMMIT. Changing activation alone would need additional gains outside those windows to reverse that recorded outcome. A new comparison would have its own duplicate spreads. Neither this calculation nor the open-transaction statistics provide a Linux CPU price for the CHECK. [Raw-ledger replay, run with `--activation`](prepared_turn_gaps.py)
