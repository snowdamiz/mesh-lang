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

Run `python3 -B constraint_budget.py` from this directory to read the archived stock FILE and Oban ledgers. The calculation checks the per-arm release records, raw hashes, input payload identities, and 128 independent application commits per arm.
