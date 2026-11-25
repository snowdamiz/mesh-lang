# Where the two LARGE FILE pilots saved time

On default storage, preparing the next producer input early moved the last durable application write forward in both rotations. Preparing claims before taking the queue lock delayed the first wave of durable writes, then recovered enough time to finish the last wave earlier. The sole controller reserved a separate experiment at 10:07 UTC to test both changes together.

Each column uses the same control throughout one rotation, selected by the fastest submit-to-final-settlement time among the two same-image duplicates. Positive values are milliseconds saved. The original advancement decisions used their own per-clock controls.

| Milestone or interval | Producer forward | Producer reverse | Claim forward | Claim reverse |
|---|---:|---:|---:|---:|
| Submit to durable enqueue | 130.887 | 146.872 | −104.551 | −71.412 |
| Enqueue commit to 8th durable effect | −17.297 | 28.857 | −176.956 | −160.693 |
| Enqueue commit to 64th durable effect | −42.351 | 24.876 | 146.687 | 115.880 |
| Submit to 64th durable effect | 88.536 | 171.748 | 42.136 | 44.468 |
| Last durable effect to final settlement | −1.443 | −0.227 | −8.023 | 0.705 |
| Submit to final settlement | 87.093 | 171.521 | 34.113 | 45.173 |

For the claim treatment, 39 and 51 effects had already committed by the time the last runner confirmed its grant. Another 67.235 and 49.007 ms elapsed before the last durable effect. The original arms cannot establish why producer time varied across claim modes or how the two changes interact.

## Replay

From the repository root, with both original archive commits available in local Git, run

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-claim-gap
python3 -B "$d/crossed_file_phases.py" | cmp - "$d/crossed-file-phases.json"
```

The replay verifies both release seals, the original advancement decisions, all 16 measured archive hashes, stock default storage, and all three scorer clocks. Each arm ran eight runners with 64 slots, submitted the same 64 payloads in 64 scalar admissions, and recorded 128 application XIDs and 64 individual terminal XIDs. Both original peer stages stayed closed. The prospective experiment has its own stock FILE bridge, duplicate controls and comparison against each individual treatment.
