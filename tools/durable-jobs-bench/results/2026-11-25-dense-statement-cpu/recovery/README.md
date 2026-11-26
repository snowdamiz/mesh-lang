# Offline recovery of the two statement CPU orders

Both orders have all 24 warmup and measured raw archives. Every adapter exited 0. The Linux PostgreSQL 17.11 build, load and exit SETUP released successfully. The original timing scorer released with exit 1 after reporting `unmapped: 3761` for group 1 and `unmapped: 2309` for group 2. The original code, receipts, log and captures remain in `c3864eb18` and `7a5427d55`. The recovery used those files with zero host dispatches. [SETUP release](../evidence/native/setup/controller-release.json) · [TIMING release](../evidence/native/timing/controller-release.json) · [Original log](../evidence/native/timing/controller.log)

PostgreSQL serialized dictionary database and effective-role OIDs as strings. The hook emitted them as JSON integers. The recovery checks each source's exact type and the nonzero unsigned 32-bit range, then converts the two dictionary fields. It rejects noncanonical numeric text and duplicate normalized keys. Signed query IDs retain their original text and are checked against the int64 range. The corrected replay applies the original raw validation, arm order, hashes, identity, CPU and perturbation checks without editing the sealed scorer.

## Original perturbation screen failed

Twelve of 16 gates failed. The screen still requires duplicate OFF spread at most 10% of the OFF midpoint. The absolute ON shift must be within 5% of that midpoint or half the duplicate spread, whichever is larger. Both orders must pass all four clocks for each engine.

| Order | Engine | Failed checks | Observed differences |
|---|---|---|---|
| 1 | Mesh | Submit, drain | OFF spread 30.83%, 34.18% |
| 1 | River | Whole-arm host CPU, whole-arm duration | ON shift +14.13%, +6.54% |
| 2 | Mesh | Submit, drain, whole-arm host CPU, whole-arm duration | OFF spread 33.40%, 38.73%, 10.24%, 10.98% |
| 2 | River | Submit, drain, whole-arm host CPU, whole-arm duration | ON shift +5.16%, +5.12%. Host CPU and duration OFF spread 27.63%, 17.33% |

## Diagnostic CPU and count evidence

The counts below refer to the named statements. The CPU columns sum their hooks across authenticated sessions for the **whole arm**. The retained counters have no hook timestamps, so they cannot isolate CPU between first submit and last settlement. The failing screen also prevents inferring normal-workload CPU from ON. Separately, Mesh session residual was 1,241 and 1,219 CPU-ms and River was 977 and 984 CPU-ms. Observer-family costs include whole-arm stock harness observation, which is a different scope from the ledger window. [Recovered output](recovered.json)

| Engine and statement | Calls in ON orders 1 / 2 | Diagnostic CPU-ms in ON orders 1 / 2 |
|---|---:|---:|
| Mesh grant | 121 / 117 | 622.1 / 574.4 |
| River grant | 103 / 105 | 74.5 / 70.3 |
| Mesh individual settlement | 1,024 / 1,024 | 358.9 / 340.2 |
| River batch settlement | 12 / 14 | 48.6 / 52.7 |
| Mesh application inserts | 2,048 / 2,048 | 100.8 / 96.9 |
| River application inserts | 2,048 / 2,048 | 119.9 / 124.8 |
| Mesh enqueue | 8 / 8 | 120.7 / 104.7 |
| River enqueue | 8 / 8 | 20.8 / 23.3 |

Across all measured OFF and ON arms, Mesh issued 113–142 grant calls and River issued 98–105. All 24 arms preserve eight enqueue statements, 2,048 application INSERTs and 1,024 reconciled jobs. Hook call vectors match the stock dictionary in all four measured ON arms. Mesh grant and settlement also match the separate queue snapshot. Similar grant call counts across engines coexist with a large measured difference in full-arm hook CPU. These captures leave the grant function's internal cost split unresolved.

The smallest change worth investigating is a bounded lookup of already-locked grant rows on the UPDATE side. If the exact hot plan shows repeated broad UPDATE visits, carry the selected tuple's `ctid` through the existing CTEs and add a bounded TID predicate in the same statement. Keep the namespace and ID checks, pick order, row locks, admission, sizing and fallback. Require a bounded TID path in both generic and custom head and mid plans and zero empty-arm UPDATE visits before any end-to-end booking. The earlier ID-membership version failed its plan gate and visited 1,024 rows on the empty shape. This is a candidate for a plan check. The CPU recovery leaves the performance gate closed. [Prior plan rejection](../../2026-11-24-public-dense-bounded-grant/evidence/native/plan-gate.json)

## Replay

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-statement-cpu/recovery
python3 -B "$d/test_replay.py"
python3 -B "$d/replay.py" | cmp - "$d/recovered.json"
python3 -B "$d/replay.py" --full
```
