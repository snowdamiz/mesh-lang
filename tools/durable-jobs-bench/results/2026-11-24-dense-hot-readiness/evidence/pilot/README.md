# Logger-off actor-readiness reversed pilot

The Linode pilot released with exit 1 at 01:19:46 UTC. Both
rotations exported all 12 raw warmup and measured arms. The first
rotation failed qualification because its measured hot-sync B stock
control recorded one checked-cursor fallback. It still recorded
1,024 distinct terminal transactions.

The second rotation reconciled. The candidate was 24.648 ms slower
on full submit and 31.598 ms slower on drain against the faster
control. Its middle and final effect clocks, and its runner CPU limit,
also missed the gate. Peer staging stayed conditional.

See the [release](controller-release.json), [replay](summary.json), and
[stock cursor receipt](group-1-cursor-observation.json). Offline replay
retains the failed first block and exits 1.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-hot-readiness/controller.py replay pilot
```
