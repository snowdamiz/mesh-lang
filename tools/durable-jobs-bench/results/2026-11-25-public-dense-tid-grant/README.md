# Same-statement physical claim target rejected

The PostgreSQL 17.11 replay reproduced a lost grant at READ COMMITTED. With two eligible rows, stock granted `[1, 2]` and the TID overlay granted `[2]`. The first row stayed eligible and the second passed it in the returned prefix. The existing function returns immediately when the grant CTE yields any row, so a partial result skips the cursor fallback. The released Rust decoder accepts a short claimed batch. The sole controller received the rejection. The treatment has no native booking. [Pinned claim SQL](../../../../compiler/mesh-rt/src/jobs/schema.sql) · [Rust claim decoder](../../../../compiler/mesh-rt/src/jobs/store.rs) · [Replay evidence](evidence.json.gz)

## The locks protect the new row version

The overlay carries the locked relation OID, `xmin`, and `ctid` through its materialized CTEs. It joins on all three plus namespace and UUID. Those physical identifiers stay inside the statement and never enter the caller's response. The existing capacity checks, claim fences, ordering, lease construction, context limits, and return shape are inherited from the pinned definition. The installer probe compares function identity, owner, ACL, security mode, settings, and volatility after replacement. [SQL patch](grant.patch)

The race replay extracts the exact stock and overlay grant statements and adds a test advisory gate to the pick predicate before `LockRows`. A second SQL session commits a replacement for row 1 once the reader waits at that gate. The reader holds the queue row and shared namespace advisory lock. A separate pick-only execution verifies that `LockRows` returned the replacement's relation, `xmin`, `ctid`, and payload. PostgreSQL's EvalPlanQual follows the committed replacement while locking. The target TID scan fetches that explicit tuple using the older command snapshot, under which the replacement is invisible. Stock reaches the replacement through the normal UPDATE retry and recheck. With only row 1 eligible, stock returned its updated context and the overlay grant statement returned zero rows. The race transactions roll back. [Replay](replay.py) · [PG17.11 LockRows](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/executor/nodeLockRows.c#L168-L262) · [PG17.11 TID fetch](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/executor/nodeTidscan.c#L304-L363) · [READ COMMITTED](https://www.postgresql.org/docs/17/transaction-iso.html#XACT-READ-COMMITTED)

The uninstrumented function replay checks an update committed before the claim starts, mixed handler versions, order, eligibility and row-version context blocks. It checks a matching UUID in a different namespace, `SKIP LOCKED`, two claimers, caps, leases, context bytes, rollback, the namespace fences, and oversized and isolation fallback. The pinned schema is an ordinary heap. The experiment declines to install the overlay for an inherited or partitioned relation. An inherited-row control claims with the unchanged stock definition. Deployment would require protecting the checked layout against concurrent DDL. The replay assumes isolated migration and DDL.

The pinned migration embeds `schema.sql` in `store.rs`. The runner checks the function signature at startup, then calls that signature through the checked set-claim path and reads the existing wire fields with ordinality. The cursor claim and next-due use the existing context-block coordinates independently of this patch. The intended native lane is public grouped hot at `b3557273d72be3802bf189b284e7caaf5f3caa0f`, with the stock ELF SHA-256 `af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355`. The local replay uses the pinned schema and SQL, and does not execute the ELF. [Runner](../../../../compiler/mesh-rt/src/jobs/runner.rs) · [Migration and wire contract](../../../../compiler/mesh-rt/src/jobs/store.rs) · [Pinned public packet](../2026-11-24-public-dense-set-claim/controller-packet.json)

## Seeded plans show where the bitmap is possible

All 15 local overlay EXPLAINs used a TID scan driven by the capped `sized` CTE. Thirteen used one parameterized TID lookup per row. The forced custom head16 and mid16 plans used one scan of the budgeted TID array. The five shapes cover head, two mid-drain counts, tail, and empty in custom, generic, and auto modes. The auto replay selected a generic plan after five custom executions. In those plans an empty `sized` CTE supplies no scan inputs, and the budget excludes empty and oversized batches from the TID array. The zero-visit bound for empty follows from the plan shape. The local runs used ordinary EXPLAIN and collected no executor counts or performance measurements. Local stock selected `jobs_pkey` for all tested shapes. [Local plans and prepared counters](evidence.json.gz)

Historical native group 0 recorded a different stock plan on seeded fixtures. The table reports actual target-side index rows per loop and loops from the raw plans. The bitmap index produces entries that the heap scan and join can later reject.

| Seeded stock shape | Auto target | Forced custom target | Forced generic target |
| --- | --- | --- | --- |
| Head, 16 slots | `jobs_pkey` 1 × 16 | Bitmap 1,032 × 16 | Bitmap 1,032 × 16 |
| Mid-drain, 16 slots | `jobs_pkey` 1 × 16 | Bitmap 1,032 × 16 | Bitmap 1,032 × 16 |
| Mid-drain, 8 slots | Bitmap 1,048 × 8 | Bitmap 1,028 × 8 | Bitmap 1,028 × 8 |
| Tail, 8 slots | Bitmap 1,046 × 8 | Bitmap 1,028 × 8 | Bitmap 1,028 × 8 |
| Empty | Zero loops | Zero loops | Zero loops |

The bitmap index is `mesh_jobs_unique_candidates`. The raw member is `adapter/grant-plans.json`. The local replay reads the stock records from that archive and preserves their target nodes, actual counts, prepared-plan counters, and member SHA-256. The failed UUID array experiment was not rerun. [Group 0 raw archive](../2026-11-24-public-dense-bounded-grant/evidence/native/blocks/00-noop.1795567469063474000/measured-qualify/measured-mesh/raw.tar.gz)

The stock live-plan owner reported that the original capture stopped in its first trace-off arm and the first recovery stopped before launch. The independently booked stock retry can resolve the remaining question. The archived real queue counters cover 147,218 and 146,252 jobs index fetches in two traced arms. Nested query ID `6827047345048828466` recorded 131,661 and 132,080 shared-buffer hits. Those totals cannot attribute index work to the UPDATE target. No treatment was launched, staged, or timed, and no treatment booking was requested. [Historical real queue](../2026-11-24-public-dense-set-claim/NATIVE-RESULTS.md) · [Stock live-plan capture](../2026-11-25-dense-live-grant-plans/evidence/native/README.md)

## Condition for a separate design

The live-plan owner has one follow-up condition. Consider a separate two-command design only if stock auto plans on the real, churned queue show that expensive UPDATE-target bitmaps occur often enough to account for material work. The first command would lock, select, and render a manifest inside the same locked READ COMMITTED transaction. The second would use a fresh command snapshot for the bounded UPDATE, verify every returned identity and the full row count, and preserve the ordering, fences, and caps. Keep the manifest within that transaction on the same backend. A partial update must roll back before any fallback can run.

A separate design must pass the exact EvalPlanQual race, mixed-version and order tests, and all fallback, clock, and transaction checks before native booking. A layout that cannot be protected, a practically cheap stock auto target, lack of material attributable query CPU and index work, or a failed correctness or access bound closes launch. A qualifying native comparison would use unchanged public producer and runner binaries, measured query-ID-specific work, and two reversed duplicate-control groups. Forced plans and profiling overhead cannot establish native throughput. This follow-up has no reservation.

## Replay

Run from the repository root with Homebrew PostgreSQL 17.11 or set `PG17_BIN` to the PostgreSQL 17.11 binary directory. Output contains full ordinary EXPLAIN plans and the archived stock actual-plan extracts. The replay stops the disposable cluster and removes its private directory.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-25-public-dense-tid-grant/replay.py
```
