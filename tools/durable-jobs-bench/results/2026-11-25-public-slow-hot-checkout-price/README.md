# Public SLOW checkout price

**Close the standalone hot-checkout screen at cap 68.** The retained native evidence supports a possible SLOW drain gain, but it does not price enough gain to close even the smallest of the three canonical stock River losses. The decision is specific to 1×64 inline, 512 × 16 KiB jobs, the 100 ms hold, eight warm application TLS sessions and individual terminal writes. No new native intake was registered.

## Same-image reference

The released dense checkout campaign used the exact public Linux `b3557273` / `af2aca867a02…` image on CPUs 0 and 1. The six paired groups used 1,024 jobs, two runners, 32 warm app sessions, cap 96 and grouped success. The two checked lanes sent 2,048 probes per arm. Hot sent four. All 36 Mesh warmup and measured archives were rehashed and their app writes, input, process identities and clocks were reconciled. The active participant sessions in the sampled ready roster ranged from four to eight. [Native comparison](../2026-11-24-dense-product-d2-transfer/NATIVE-RESULTS.md) · [Replayed counts and clocks](evidence/prices.json)

Relative to the faster checked arm on each clock, hot saved a median **113.4 ms from submit**, **115.3 ms on drain**, and **115.4 ms to the last committed effect after enqueue**. The drain gains ranged from 94.7 to 142.9 ms. The projection divides each dense drain gain by its 2,044 avoided probes and credits SLOW with avoiding all 1,024. That gives **47.4–71.6 ms**, median **57.8 ms**, while holding the producer fixed. This is a transfer estimate. Dense has no 100 ms hold and uses a different runner shape, app pool width and terminal topology. Neither SQL execution time nor the dense elapsed contrast directly times the client round trip in SLOW.

## SLOW waves and required gain

The five canonical measured arms and all eight cold inline pilot archives had 1,024 probes, 1,024 independent app XIDs, 512 individual terminal XIDs and a real hold of at least 100 ms per job. The canonical PostgreSQL probe statements used **2.53–3.89 ms** of cumulative execution time per arm. That excludes planning, the TLS exchange, client work, scheduling and any queueing induced by the probe. The first durable write’s 64th-to-512th rank interval lasted **1,097–1,217 ms**. The final durable effect preceded the final settlement by **1.3–6.3 ms**.

The fixed holds alone occupy 51,200 slot-ms, equivalent to 800 ms at full use of 64 seats. Across the canonical runs, the entire handler-entry-to-first-commit interval plus time above the 100 ms hold contributed **56.2–61.2 slot-normalized ms**. This is summed job time divided by 64. It includes both writes’ non-hold exposure and other client, scheduler and database work. Rank waves and slot-normalized time characterize the ledger and do not put a hard upper bound on a different execution schedule. Probe elimination can also change contention and when seats refill.

| Canonical block | Submit gap to River | Existing drain gap | Drain saving needed with the producer fixed | Gap after the best dense-rate projection |
| --- | ---: | ---: | ---: | ---: |
| 1 | 94.909 ms | 37.106 ms | 94.909 ms | 23.338 ms |
| 2 | 183.206 ms | 150.129 ms | 183.206 ms | 111.635 ms |
| 5 | 96.569 ms | 58.549 ms | 96.569 ms | 24.998 ms |

A tie in those blocks would require **92.7, 178.9 and 94.3 µs** of elapsed drain gain for every removed probe, even with every probe eliminated. The best observed dense drain rate was **69.9 µs**. The recent cold inline duplicates, which used the same runner with the retained FILE-capable broker in inline mode, differed by **17.2 and 107.4 ms** on submit and **28.9 and 103.8 ms** on drain. One of the two observed duplicate spreads exceeds even the best price on both clocks. Mesh already led Oban in all five canonical submit comparisons. [SLOW and dense raw replay inputs](evidence/prices.json) · [Cold pilot source](../2026-11-25-public-slow-file-topology/roster-retry/evidence/native/pilot/replay.json)

## Opt-in and prior coverage

The exact stock runner takes its app pool URL from `MESH_EFFECT_APP_URL`. Adding `&mesh_pool_hot_checkout=1` enables the existing pool option independently of checked set claims and the terminal policy. It skips the preflight query for a recently checked-in connection under 500 ms. Older idle sockets still get a health probe, and a recently silent connection can wait on the application statement. The retained public adapter couples this option to `product-d2`, which also enables grouped settlement. Stock SLOW selects `product-bulk`, with checked set claims and scalar terminal settlement. A different ELF would be unnecessary. An exact scalar-hot screen would need the URL option applied independently and a bound scalar-hot native proof. The existing same-image native lost application reply and successful replacement evidence covers the grouped hot path. [Exact source and native proof binding](evidence/prices.json)

The archive and controller inbox scan found no prior exact 1×64 cap-68 scalar-hot SLOW booking. The two documents with both SLOW and hot indicators were the all-profile plan, whose hot lane belongs to short, and the campaign pause record, which references separate D2 work. The existing SLOW FILE topology close and the cap-71 fused hold stay in force. [Intake scan](evidence/booking-scan.json)

## Replay

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-slow-hot-checkout-price
python3 -B "$d/price.py" | cmp - "$d/evidence/prices.json"
python3 -B "$d/price.py" scan
(cd "$d" && shasum -a 256 -c SHA256SUMS)
```

The scan reads the current local inbox, so its counts can change. The pricing replay reads retained archives without contacting Linode or starting a workload.
