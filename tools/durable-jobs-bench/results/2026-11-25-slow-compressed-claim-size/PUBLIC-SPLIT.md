# Public SLOW under LZ4 with two 32-slot runners

The public set-based runner received 78 fallback markers in the forward measurement and 104 in reverse. The earlier 1×64 correctness arm recorded 34. The split measurements expose more repeated admission work than the original pricing assumed. The measured set route still finished between the two cursor controls in each order. The archived public pilot closed before peer enrollment.

| Set route, measured | Forward | Reverse |
|---|---:|---:|
| Set calls | 125 | 145 |
| Fallback followed by checked cursor | 78 | 104 |
| Empty followed by cutoff-aware due probe | 47 | 41 |
| Committed claim transactions | 203 | 249 |
| Aggregate set SQL across both runners (ms) | 305.0 | 384.4 |
| Aggregate cursor SQL across both runners (ms) | 285.5 | 299.2 |
| Submit to enqueue commit (ms) | 356.291 | 374.596 |
| Bulk and scalar writer execution (ms) | 175.606 | 184.883 |
| Remaining enqueue time (ms) | 180.685 | 189.713 |
| Enqueue commit to settlement (ms) | 1282.868 | 1300.245 |

The counts are inferred from disjoint runtime branches. In the public source at `b3557273`, `read_batch_claim` in set-based mode reaches `claim_batch_cursor_checked` only after receiving a fallback. Only the empty response carrying a cutoff reaches `next_due_ms_since`. Each such call supplies a lower bound of one on the respective branch count. Their sum accounts for every completed set call in all four set arms, including warmups. BEGIN and COMMIT counts equal the set and cursor call counts combined. All 512 stored arguments had LZ4 compression. This identifies fallback responses without attributing individual cursor grants, statement durations or job IDs to them.

The public-only replay also checks all eight bulk statements covering 504 rows and eight scalar writes covering the remaining eight per arm. In the reverse cursor controls, the slower control actually committed enqueue **8.193 ms earlier** and finished drain **327.972 ms later**. Across the six measured public arms the two writer statements accounted for 161–210 ms of enqueue. The remaining 166–190 ms includes other SQL, client work and waits. Even eliminating a producer stage cannot directly close the reverse drain gap with the observed consumer schedule.

The qualified 1×64 compressed-size guard is a candidate for pricing on these two runners. A guarded grant would still execute its size and layout checks inside the queue lock. The full set and cursor SQL totals include waits, empty scans, real grants, and overlapping sessions. Differences in these totals cannot establish a wall-clock saving. A useful deployment needs a repeatable middle-write advance with the same binary, stock set and both cursor controls. The original public peer decision and the separate six-order V1 rejection still stand.

The [reducer](public_split.py) loads the twelve public arms from Git, checks their hashes against the exit-zero [pilot release](../../../../../t3-02745283/tools/durable-jobs-bench/results/2026-11-25-public-slow-lz4-2x32-transfer/evidence/pilot/controller-release.json), reconciles all 512 jobs and 1,536 XIDs per arm, and reproduces the [per-arm receipt](public-split.json). Run `python3 -B public_split.py | cmp - public-split.json` from this directory.
