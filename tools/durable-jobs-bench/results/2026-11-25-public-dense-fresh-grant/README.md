# Fresh-snapshot claim fallback

This isolated SQL overlay fixes the original PostgreSQL 17.11 physical-target race. Stock and the fresh-command overlay both granted `[1]` and `[1, 2]` in the original two races, with the concurrently updated context on row 1. The earlier same-statement physical target remains closed. The fresh-command overlay is held as a fallback while the separate logical-key treatment is evaluated. [Local receipt](evidence.json.gz) · [Original rejection](../2026-11-25-public-dense-tid-grant/README.md)

## Same transaction, fresh command snapshot

The generator requires the exact released public `claim_batch_set_v1` definition from `b3557273d72be3802bf189b284e7caaf5f3caa0f`. The overlay retains its signature, invoker privileges, function settings, cutoff, capacity and namespace locks, pick, context encoding and size limits. It renders each candidate once. The first command collects the budgeted ordered manifest in function-local arrays. The existing budget clock provides a single common lease start after rendering. The manifest holds at most 16 entries and respects the existing wire budget before copying bodies into its array. [Generator](build_overlay.py) · [SQL fragments](fresh.sql)

The second command executes one set-based `UPDATE ONLY` using the locked TID array and checks namespace, table OID, UUID, and `xmin` against the matching manifest entry. In a VOLATILE PL/pgSQL function at READ COMMITTED, the command obtains a fresh snapshot. The locks stay held and the new snapshot can see the row version acquired by EvalPlanQual. No physical coordinate enters the response or a database table. The update returns identities and checks queue, state, attempt, token, and lease timestamps. An identity, count, or grant-field mismatch aborts the call and rolls back all its changes. After verification, the function returns the manifest in its original order. [PostgreSQL volatility and snapshots](https://www.postgresql.org/docs/17/xfunc-volatility.html)

The empty and oversize paths execute one zero-row UPDATE, so statement triggers still fire once. The existing eligibility probe supplies the busy or sizing fallback versus the cutoff-bearing empty marker. The exact released claim CTE handles row-security, inherited and partitioned tables, unsupported table access methods or rewrite rules, and disabled TID scans. The normal pick includes descendants. Its relation check routes a selected child to the released CTE if a child was attached after the catalog check. The capacity query holds a relation lock against heap rewrites. Isolation or client encoding mismatches keep the released fallback marker.

## Evidence and run status

The reconciled group 1 stock arm contains 25 nonempty plans for nested query ID `6827047345048828466`. Every target `jobs_pkey` scan read 1,024 namespace rows once, and the nested loop applied UUID equality as a join filter. The sum is 25,600 target rows. Group 2 stopped before the sampled workload. The capture establishes the real target path and does not qualify an observer-adjusted timing comparison. [Sampled raw archive](../2026-11-25-dense-live-grant-plans/procfd-recovery/evidence/native/blocks/01-noop/sampled/measured-mesh/raw.tar.gz) · [Release and reconciliation](../2026-11-25-dense-live-grant-plans/procfd-recovery/evidence/native/README.md)

Local correctness ran on a disposable PostgreSQL 17.11 cluster against the stock schema and the generated overlay. The two original races committed new row versions after the pick snapshot and before the row lock. A separate handler/version-change race returned `[2]` in both variants, with row 1 still available. The regular mixed-handler case returned both supported versions in order. The post-pick barrier verified that an outside writer and cancellation cannot move the locked row, the legacy single claimant skips the held queue, and another namespace remains writable. An injected write on the same transaction invalidated the manifest and rolled back the entire call. A separate row-trigger suppression test rolled back both the partial update and its statement trigger's write. The receipt also covers queue capacity, two claimers, namespace fences, rollback, explicit and implicit cutoff clocks, lease, context, and ordinary fallback.

All 15 ordinary EXPLAINs of the extracted update used one TID scan with the directly bound array. They cover custom, generic, and auto with head, mid-drain, tail, and empty shapes on synthetic heap churn. The empty array supplies zero TIDs. The replay uses EXPLAIN without ANALYZE and collects no runtime counts or wall or CPU data for the overlay. The native producer and ELF have not been run with it. [Replay](replay.py) · [Local receipt](evidence.json.gz)

The sole controller has the hold. There is no native packet or reservation for the fallback. A later pilot would require a logical-key disposition, exact stock producer and ELF on the matched 2×32 workload, sampled target qualification kept outside the throughput comparison, and two reversed duplicate-control groups. A peer comparison depends on both end-to-end clock and CPU gates. The extra SPI work and enqueue stay inside those comparisons.

## Replay

Run from the repository root with PostgreSQL 17.11 installed. For a non-Homebrew installation, set `PG17_BIN` to the binary directory. The script stops and removes its disposable cluster.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-25-public-dense-fresh-grant/replay.py
```
