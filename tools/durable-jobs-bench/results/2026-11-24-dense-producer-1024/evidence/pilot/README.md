# Private producer-width timing entry failure

The pilot controller released both requested rotations with exit 1.
All 12 attempted arms exported failure receipts. The adapter stopped
before workload while searching the frozen harness for a textual
source anchor. The qualified compiled private broker remains bound
to the earlier SETUP. Replay selects no comparison block.

See the [release](controller-release.json), [entry traceback](entry-failure.txt),
and [replay](summary.json). Peer intake remains conditional.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/controller.py replay pilot
```
