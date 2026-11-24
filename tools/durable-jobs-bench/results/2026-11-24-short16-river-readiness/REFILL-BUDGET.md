# How much can earlier refill alone save?

In six matched Linode short16 rotations, hot-checkout Mesh took a paired median 39.96 ms longer than prewarmed River from enqueue commit to settlement. Its 16th committed write pair arrived only 1.74 ms later. The next 112 pairs took 66.54 ms longer. These intervals include service inside the handler and waits for runner capacity.

To price idle slots between claims, replay the saved claim cohorts in their original order on 16 slots. Start each replacement cohort as soon as enough of its predecessors have durably settled. Keep the first claim at its observed time, keep each job's time from its claim to terminal commit, and preserve its entry and write offsets by translating the entire timeline. The original SQL marker is truncated to milliseconds. One scenario uses the earliest feasible timestamp in its bucket. The second uses the latest timestamp consistent with handler entry. Both check the slot cap and completion ordering.

| Paired median elapsed difference | Early timestamps | Late timestamps |
|---|---:|---:|
| Mesh time saved by instantaneous refill | 11.15 ms | 19.32 ms |
| Remaining Mesh deficit to warm River from submit | 48.23 ms | 40.14 ms |
| Remaining Mesh deficit to warm River from commit | 27.85 ms | 19.75 ms |
| Remaining Mesh deficit to Apalis from commit | 21.24 ms | 12.41 ms |

Under either scenario Mesh still lost four of the six drain comparisons to warm River and five to Apalis. Within these observed batch sizes, matching warm River consistently would also require changing residence after the claim, or a different cohort size or ordering. These are counterfactuals. The remaining differences depend on the assumption that earlier launches preserve measured service times under contention. Fused refill could change those times and needs its own native result.

Even allowing arbitrary cohort sizes and job ordering, there is a lower bound from the longest residence and total slot time divided by 16. Use the earliest first claim and the latest feasible claim on every job to make this optimistic. With the original producer and fixed residence, this floor still trails warm River from submit in groups 1, 2, 3, and 5 by 10.98, 13.55, 61.99, and 30.34 ms. From commit the corresponding floor exceeds warm River in groups 3 and 5 by 26.28 and 1.91 ms. A floor below the comparator leaves feasibility open.

Run the following to reconstruct both scenarios from the six archived Mesh ledgers and verify each raw digest against the reconciled comparison. [Per-rotation differences](evidence/refill-budget.json) · [Measured stage differences](NATIVE-RESULTS.md)

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-short16-river-readiness/refill_budget.py
```
